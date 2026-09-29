// SPDX-License-Identifier: AGPL-3.0-only
//! The address bar's protocol chooser (NET-029) and recent servers
//! (NET-019).
//!
//! Dolphin's editable location bar offers the protocols it can browse
//! when the address is empty. Here, emptying the address shows a list
//! under the entry: SMB, SFTP, FTP, FTPS, WebDAV, secure WebDAV and NFS,
//! then the servers recently connected in OpenXplorer, Files or the GTK
//! file chooser ([`RecentServers`]). Picking a protocol types `scheme://`
//! and the user goes on typing the server; picking a server types its
//! address. The list never takes the keyboard focus, so typing continues
//! in the entry, and it hides as soon as the address has text again.

use gtk::glib;
use gtk::prelude::*;
use ox_core::network::RecentServers;

use crate::dialogs::Protocol;

/// Adds the chooser to `entry` and returns it; the caller unparents it
/// when `entry` goes away.
pub(super) fn protocol_chooser(entry: &gtk::Entry) -> gtk::Popover {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let popover = gtk::Popover::builder()
        .autohide(false)
        .has_arrow(false)
        .position(gtk::PositionType::Bottom)
        .halign(gtk::Align::Start)
        .child(&list)
        .css_classes(["ox-menu", "classic"])
        .build();
    for protocol in Protocol::ALL {
        let prefix = format!("{}://", protocol.scheme());
        list.append(&choice_button(protocol.label(), &prefix, entry, &popover));
    }
    let recent = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.append(&recent);
    popover.set_parent(entry);
    let show = move |entry: &gtk::Entry, popover: &gtk::Popover| show_when_empty(entry, popover, &recent);
    let show = std::rc::Rc::new(show);
    entry.connect_changed(glib::clone!(
        #[weak]
        popover,
        #[strong]
        show,
        move |entry| show(entry, &popover)
    ));
    let focus = gtk::EventControllerFocus::new();
    focus.connect_enter(glib::clone!(
        #[weak]
        popover,
        move |focus| {
            if let Some(entry) = focus.widget().and_downcast::<gtk::Entry>() {
                show(&entry, &popover);
            }
        }
    ));
    focus.connect_leave(glib::clone!(
        #[weak]
        popover,
        move |_| popover.popdown()
    ));
    entry.add_controller(focus);
    popover
}

/// Shows the chooser, with the recent servers read afresh, while the
/// shown entry is empty; else hides it.
fn show_when_empty(entry: &gtk::Entry, popover: &gtk::Popover, recent: &gtk::Box) {
    if !entry.text().is_empty() || !entry.is_mapped() {
        popover.popdown();
        return;
    }
    if !popover.is_visible() {
        fill_recent_servers(recent, entry, popover);
    }
    popover.popup();
}

/// Replaces the recent servers under the protocols.
fn fill_recent_servers(recent: &gtk::Box, entry: &gtk::Entry, popover: &gtk::Popover) {
    while let Some(child) = recent.first_child() {
        recent.remove(&child);
    }
    let servers = RecentServers::for_user().suggestions();
    if servers.is_empty() {
        return;
    }
    let heading = gtk::Label::builder()
        .label("Recent servers")
        .xalign(0.0)
        .css_classes(["dim-label", "caption"])
        .build();
    recent.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    recent.append(&heading);
    for server in servers {
        recent.append(&choice_button(&server, &server, entry, popover));
    }
}

/// A line of the chooser reading `label`, which types `text`.
fn choice_button(label: &str, text: &str, entry: &gtk::Entry, popover: &gtk::Popover) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let name = gtk::Label::builder().label(label).hexpand(true).xalign(0.0).build();
    content.append(&name);
    if label != text {
        content.append(&gtk::Label::builder().label(text).css_classes(["dim-label"]).build());
    }
    let button = gtk::Button::builder()
        .child(&content)
        .focus_on_click(false)
        .can_focus(false)
        .css_classes(["flat"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    let text = text.to_owned();
    button.connect_clicked(glib::clone!(
        #[weak]
        entry,
        #[weak]
        popover,
        move |_| {
            popover.popdown();
            entry.set_text(&text);
            entry.grab_focus_without_selecting();
            entry.set_position(-1);
        }
    ));
    button
}
