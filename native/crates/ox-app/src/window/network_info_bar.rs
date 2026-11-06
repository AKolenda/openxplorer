// SPDX-License-Identifier: AGPL-3.0-only
//! Windows' info bar on Network while discovery cannot run.
//!
//! Windows 11 lists Network like a folder, and when network discovery is
//! off it shows a yellow bar above the items: "Network discovery is
//! turned off. Network computers and devices are not visible. Click to
//! change...". The app cannot turn discovery on by itself: it reads
//! `GVfs`'s network browser, which needs the desktop's `GVfs` network
//! services. So the bar says Windows' first two sentences when the browser
//! cannot be read, gives GIO's own reason as its tooltip, and opens Help
//! at "Network locations", which says what discovery needs, when it is
//! clicked or activated with the keyboard. It spans the folder panes, as
//! Windows' bar spans the item view, and hides once discovery runs again
//! or another location is in front.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::locations::Page;

use super::window_action::WindowAction;
use super::BrowserWindow;

/// What the bar says (Windows 11's words, then where clicking leads).
const DISCOVERY_OFF: &str = crate::i18n::message_id(
    "Network discovery is turned off. Network computers and devices are not visible. Click to learn more...",
);

/// The bar: a flat, full-width button with its text at the start.
fn info_bar() -> gtk::Button {
    let text = gtk::Label::builder()
        .label(ox_core::i18n::gettext_static(DISCOVERY_OFF))
        .xalign(0.0)
        .wrap(true)
        .build();
    gtk::Button::builder()
        .child(&text)
        .action_name(WindowAction::Help.detailed_name())
        .hexpand(true)
        .visible(false)
        .css_classes(["info-bar"])
        .build()
}

impl BrowserWindow {
    /// Network's info bar.
    pub(super) fn network_info_bar(&self) -> &gtk::Button {
        self.imp()
            .network_info_bar
            .get()
            .expect("BrowserWindow::new installs the info bar")
    }

    /// Puts the info bar above the folder panes, and the search strip.
    pub(super) fn install_network_info_bar(&self) {
        let bar = info_bar();
        if let Some(column) = self.search_strip().parent().and_downcast::<gtk::Box>() {
            column.prepend(&bar);
        }
        self.imp().network_info_bar.set(bar).expect("installed once");
    }

    /// Shows the info bar while Network is in front and its last listing
    /// could not discover computers; hides it otherwise.
    pub(super) fn show_network_info_bar(&self) {
        let on_network = self
            .current_uri()
            .is_some_and(|uri| Page::from_uri(&uri) == Some(Page::Network));
        let problem = on_network.then(|| self.network().discovery_problem()).flatten();
        let bar = self.network_info_bar();
        bar.set_tooltip_text(problem.as_deref());
        bar.set_visible(problem.is_some());
    }
}
