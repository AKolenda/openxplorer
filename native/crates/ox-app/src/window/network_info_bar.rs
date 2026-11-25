// SPDX-License-Identifier: AGPL-3.0-only
//! Windows' info bar on Network while discovery cannot run.
//!
//! Windows 11 lists Network like a folder, and when network discovery is
//! off it shows a yellow bar under the command bar: "Network discovery
//! and file sharing are turned off. Network computers and devices are not
//! visible. Click to change...", with an × at the right. Clicking it
//! opens a menu: "Turn on network discovery and file sharing" and "Open
//! Network and Sharing Center".
//!
//! The app reads `GVfs`'s network browser, which needs the desktop's own
//! discovery services (Avahi for DNS-SD, wsdd for WS-Discovery), and it
//! never starts or changes a system service by itself (AGENTS.md: native
//! actions stay opt-in). So the bar shows when the browser is missing or
//! cannot be read, or when neither service is there and nothing was
//! found; its tooltip says which, and its menu offers "How to turn on
//! network discovery", the manual's steps, and Help. The × hides it until
//! discovery runs again. It spans the folder panes, as Windows' bar spans
//! the item view, and hides once discovery runs again or another location
//! is in front.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::icons::Icon;
use crate::locations::Page;

use super::menu_popover::{MenuEntry, MenuItem, MenuPopover};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// What the bar says, in Windows 11's words.
const DISCOVERY_OFF: &str = crate::i18n::message_id(
    "Network discovery is turned off. Network computers and devices are not visible. Click to change...",
);

/// The bar's menu, as Windows' bar opens one.
fn bar_menu() -> Vec<MenuEntry> {
    vec![
        MenuItem::new(
            &ox_core::i18n::gettext("How to turn on network discovery"),
            Icon::ShieldLock,
            WindowAction::NetworkDiscoveryHelp,
        )
        .into(),
        MenuItem::new(
            &ox_core::i18n::gettext("Open Help"),
            Icon::Info,
            WindowAction::Help,
        )
        .into(),
    ]
}

/// The parts of the bar.
#[derive(Debug)]
pub(super) struct NetworkInfoBar {
    /// The whole bar.
    pub(super) bar: gtk::Box,
    /// The text, a flat button that opens the menu.
    pub(super) text: gtk::Button,
}

/// The bar: its text, a full-width flat button that opens the menu, then
/// the × that hides it.
fn info_bar() -> NetworkInfoBar {
    let label = gtk::Label::builder()
        .label(ox_core::i18n::gettext_static(DISCOVERY_OFF))
        .xalign(0.0)
        .wrap(true)
        .build();
    let text = gtk::Button::builder()
        .child(&label)
        .hexpand(true)
        .css_classes(["info-bar-text"])
        .build();
    let close = gtk::Button::builder()
        .child(&crate::icons::image(Icon::Dismiss, 16))
        .tooltip_text(ox_core::i18n::gettext("Close"))
        .action_name(WindowAction::DismissNetworkInfoBar.detailed_name())
        .valign(gtk::Align::Center)
        .css_classes(["info-bar-close"])
        .build();
    close.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext("Close"))]);
    let bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .visible(false)
        .css_classes(["info-bar"])
        .build();
    bar.append(&text);
    bar.append(&close);
    NetworkInfoBar { bar, text }
}

impl BrowserWindow {
    /// Network's info bar.
    pub(super) fn network_info_bar(&self) -> &NetworkInfoBar {
        self.imp()
            .network_info_bar
            .get()
            .expect("BrowserWindow::new installs the info bar")
    }

    /// Puts the info bar above the folder panes, and the search strip.
    pub(super) fn install_network_info_bar(&self) {
        let parts = info_bar();
        if let Some(column) = self.search_strip().parent().and_downcast::<gtk::Box>() {
            column.prepend(&parts.bar);
        }
        parts.text.connect_clicked(Self::popup_network_info_menu);
        self.imp().network_info_bar.set(parts).expect("installed once");
    }

    /// Opens the bar's menu under its text.
    fn popup_network_info_menu(text: &gtk::Button) {
        let popover = MenuPopover::new(bar_menu());
        popover.set_parent(text);
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_halign(gtk::Align::Start);
        popover.connect_closed(|popover| {
            let closed = popover.clone();
            glib::idle_add_local_once(move || closed.unparent());
        });
        popover.popup();
    }

    /// Hides the bar until discovery runs again (its ×).
    pub(super) fn dismiss_network_info_bar(&self) {
        self.network().dismiss_discovery_problem();
        self.show_network_info_bar();
    }

    /// Shows the info bar while Network is in front and its last listing
    /// could not discover computers, unless its × hid it; hides it
    /// otherwise.
    pub(super) fn show_network_info_bar(&self) {
        let on_network = self
            .current_uri()
            .is_some_and(|uri| Page::from_uri(&uri) == Some(Page::Network));
        let problem = on_network
            .then(|| self.network().shown_discovery_problem())
            .flatten();
        let parts = self.network_info_bar();
        parts.text.set_tooltip_text(problem.as_deref());
        parts.bar.set_visible(problem.is_some());
    }
}
