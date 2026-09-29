// SPDX-License-Identifier: AGPL-3.0-only
//! The address bar's protocol chooser (NET-029).
//!
//! Dolphin's editable location bar offers the protocols it can browse
//! when the address is empty. Here, emptying the address shows a list
//! under the entry: SMB, SFTP, FTP, FTPS, WebDAV, secure WebDAV and NFS.
//! Picking one types `scheme://` and the user goes on typing the server.
//! The list never takes the keyboard focus, so typing continues in the
//! entry, and it hides as soon as the address has text again.

use gtk::glib;
use gtk::prelude::*;

use crate::dialogs::Protocol;

/// Adds the chooser to `entry` and returns it; the caller unparents it
/// when `entry` goes away.
pub(super) fn protocol_chooser(entry: &gtk::Entry) -> gtk::Popover {
    let list = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .build();
    let popover = gtk::Popover::builder()
        .autohide(false)
        .has_arrow(false)
        .position(gtk::PositionType::Bottom)
        .halign(gtk::Align::Start)
        .child(&list)
        .css_classes(["ox-menu", "classic"])
        .build();
    for protocol in Protocol::ALL {
        list.append(&protocol_button(protocol, entry, &popover));
    }
    popover.set_parent(entry);
    entry.connect_changed(glib::clone!(
        #[weak]
        popover,
        move |entry| show_when_empty(entry, &popover)
    ));
    let focus = gtk::EventControllerFocus::new();
    focus.connect_enter(glib::clone!(
        #[weak]
        popover,
        move |focus| {
            if let Some(entry) = focus.widget().and_downcast::<gtk::Entry>() {
                show_when_empty(&entry, &popover);
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

/// Shows the chooser while the shown entry is empty, else hides it.
fn show_when_empty(entry: &gtk::Entry, popover: &gtk::Popover) {
    let is_empty = entry.text().is_empty();
    if is_empty && entry.is_mapped() {
        popover.popup();
    } else {
        popover.popdown();
    }
}

/// A line of the chooser: the protocol's name and its `scheme://`.
fn protocol_button(protocol: Protocol, entry: &gtk::Entry, popover: &gtk::Popover) -> gtk::Button {
    let prefix = format!("{}://", protocol.scheme());
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&gtk::Label::builder().label(protocol.label()).hexpand(true).xalign(0.0).build());
    content.append(&gtk::Label::builder().label(&prefix).css_classes(["dim-label"]).build());
    let button = gtk::Button::builder()
        .child(&content)
        .focus_on_click(false)
        .can_focus(false)
        .css_classes(["flat"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(protocol.label())]);
    button.connect_clicked(glib::clone!(
        #[weak]
        entry,
        #[weak]
        popover,
        move |_| {
            popover.popdown();
            entry.set_text(&prefix);
            entry.grab_focus_without_selecting();
            entry.set_position(-1);
        }
    ));
    button
}
