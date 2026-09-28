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
use ox_core::location::{self, is_device_location, parent_location, LocationContext};

use crate::icons::{ArtKind, Glyph};
use crate::locations::Page;

use super::address_bar::{AddressIcon, CrumbButton};
use super::location_kind::is_smb_location;
use super::session::{Session, Tab};
use super::tab_strip::{TabIcon, TabView};
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
fn address_icon(uri: &str) -> AddressIcon {
    if let Some(page) = Page::from_uri(uri) {
        return AddressIcon::Glyph(page.glyph());
    }
    if is_smb_location(uri) {
        AddressIcon::Glyph(Glyph::Network)
    } else if is_device_location(uri) {
        AddressIcon::Glyph(Glyph::Phone)
    } else {
        AddressIcon::Folder
    }
}

/// A tab's icon, as `renderTabs` picks it: the network glyph on the
/// Network page, a phone for devices, network art for SMB, and the colour
/// folder everywhere else, This PC included.
fn tab_icon(uri: &str) -> TabIcon {
    if Page::from_uri(uri) == Some(Page::Network) {
        return TabIcon::Glyph(Glyph::Network);
    }
    if is_device_location(uri) {
        TabIcon::Glyph(Glyph::Phone)
    } else if is_smb_location(uri) {
        TabIcon::Art(ArtKind::NetworkFolder)
    } else {
        TabIcon::Art(ArtKind::Folder)
    }
}

/// How the strip shows `tab`: its title, its address (with "Network
/// location" for SMB) and its icon.
fn tab_view(tab: &Tab, session: &Session, locations: &LocationContext) -> TabView {
    let uri = tab.uri();
    let mut tooltip = locations.display_location(uri);
    if is_smb_location(uri) {
        tooltip.push_str(" · Network location");
    }
    TabView {
        id: tab.id,
        title: locations.title_for(uri),
        tooltip,
        icon: tab_icon(uri),
        active: session.is_active(tab.id),
    }
}

impl BrowserWindow {
    /// Updates the frame after the active tab moved: the address bar
    /// returns to breadcrumbs.
    pub(super) fn render_navigation(&self) {
        self.render_location();
        if let Some(uri) = self.current_uri() {
            let address = self.imp().locations.borrow().display_location(&uri);
            self.chrome().address.show_crumbs(&address);
        }
    }

    /// Updates the window title, history buttons, breadcrumbs, tabs,
    /// sidebar highlight and landing page for the active tab's location.
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
        let search = &self.chrome().search;
        search.set_folder_title(&title);
        search.set_enabled(!on_page && !is_device_location(uri));
        self.render_tabs();
        self.sidebar().select(uri);
        self.render_landing();
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
                divider_before: location::crumb_divider(uri, &breadcrumbs, index),
                crumb: crumb.clone(),
            })
            .collect();
        let address = locations.display_location(uri);
        self.chrome()
            .address
            .show_location(&crumbs, &address, address_icon(uri), self.art_style());
    }

    /// Redraws the tab strip.
    pub(super) fn render_tabs(&self) {
        let views: Vec<TabView> = {
            let session = self.imp().session.borrow();
            let locations = self.imp().locations.borrow();
            let tab_views = session
                .tabs()
                .iter()
                .map(|tab| tab_view(tab, &session, &locations));
            tab_views.collect()
        };
        self.chrome().tabs.show(&views, self.art_style());
    }

    /// Replaces the breadcrumbs with the editable address (Ctrl+L).
    pub(super) fn edit_address(&self) {
        let Some(uri) = self.current_uri() else { return };
        let address = self.imp().locations.borrow().display_location(&uri);
        self.chrome().address.edit(&address);
    }

    /// Ends editing with Enter or Escape: back to the breadcrumbs, with
    /// keyboard focus in the folder view.
    pub(super) fn finish_address(&self) {
        if let Some(uri) = self.current_uri() {
            let address = self.imp().locations.borrow().display_location(&uri);
            self.chrome().address.show_crumbs(&address);
        }
        self.content().focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A location and the tab and address-bar icons it shows.
    struct IconCase {
        uri: &'static str,
        tab: TabIcon,
        address: AddressIcon,
    }

    /// parity: TAB-010
    #[test]
    fn tabs_and_the_address_bar_show_the_current_apps_icons() {
        let cases = [
            IconCase {
                uri: "file:///tmp/work",
                tab: TabIcon::Art(ArtKind::Folder),
                address: AddressIcon::Folder,
            },
            IconCase {
                uri: "smb://nas/media",
                tab: TabIcon::Art(ArtKind::NetworkFolder),
                address: AddressIcon::Glyph(Glyph::Network),
            },
            IconCase {
                uri: "mtp://%5Busb%3A001%2C010%5D/",
                tab: TabIcon::Glyph(Glyph::Phone),
                address: AddressIcon::Glyph(Glyph::Phone),
            },
            IconCase {
                uri: Page::ThisPc.uri(),
                tab: TabIcon::Art(ArtKind::Folder),
                address: AddressIcon::Glyph(Glyph::Desktop),
            },
            IconCase {
                uri: Page::Network.uri(),
                tab: TabIcon::Glyph(Glyph::Network),
                address: AddressIcon::Glyph(Glyph::Network),
            },
        ];
        for case in cases {
            assert_eq!(tab_icon(case.uri), case.tab, "{}", case.uri);
            assert_eq!(address_icon(case.uri), case.address, "{}", case.uri);
        }
    }
}
