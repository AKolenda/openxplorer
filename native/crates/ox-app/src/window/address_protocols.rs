// SPDX-License-Identifier: AGPL-3.0-only
//! The address bar's protocol chooser (NET-029) and recent servers
//! (NET-019).
//!
//! Dolphin's editable location bar offers the protocols it can browse
//! when the address is empty. Here, emptying the address shows a list
//! under the entry: SMB, SFTP, FTP, FTPS, WebDAV, secure WebDAV and NFS,
//! then the servers recently connected in `OpenXplorer`, Files or the GTK
//! file chooser ([`RecentServers`]). Picking a protocol types `scheme://`
//! and the user goes on typing the server; picking a server types its
//! address. Clicking a line leaves the keyboard focus in the entry, so
//! typing continues there; Down moves into the list, whose lines Up and
//! Down walk and Enter picks, and Escape returns to the entry. The list
//! hides as soon as the address has text again.

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::dialogs::Protocol;
use crate::network::user_recent_servers;

/// Adds the chooser to `entry` and returns it; the caller unparents it
/// when `entry` goes away.
pub(super) fn protocol_chooser(entry: &gtk::Entry) -> gtk::Popover {
    let list = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .accessible_role(gtk::AccessibleRole::List)
        .build();
    list.update_property(&[gtk::accessible::Property::Label("Protocols and recent servers")]);
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
    entry.add_controller(keyboard_into_list(&popover, &list));
    popover
}

/// Down in the entry moves the focus to the chooser's first line while
/// it is shown; Escape in the list returns to the entry.
fn keyboard_into_list(popover: &gtk::Popover, list: &gtk::Box) -> gtk::EventControllerKey {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(
        #[weak]
        popover,
        #[weak]
        list,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            if key == gtk::gdk::Key::Down && popover.is_visible() && focus_first_choice(&list) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    ));
    let back = gtk::EventControllerKey::new();
    back.connect_key_pressed(glib::clone!(
        #[weak]
        popover,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            if key != gtk::gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            if let Some(entry) = popover.parent() {
                entry.grab_focus();
            }
            glib::Propagation::Stop
        }
    ));
    popover.add_controller(back);
    keys
}

/// Focuses the chooser's first line; false when it has none.
pub(super) fn focus_first_choice(list: &gtk::Box) -> bool {
    list.first_child().is_some_and(|first| first.grab_focus())
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

/// Replaces the recent servers under the protocols, read off the main
/// thread.
fn fill_recent_servers(recent: &gtk::Box, entry: &gtk::Entry, popover: &gtk::Popover) {
    while let Some(child) = recent.first_child() {
        recent.remove(&child);
    }
    let Some(lists) = user_recent_servers() else {
        return;
    };
    let (recent, entry, popover) = (recent.downgrade(), entry.downgrade(), popover.downgrade());
    glib::spawn_future_local(async move {
        let servers = gio::spawn_blocking(move || lists.suggestions())
            .await
            .unwrap_or_default();
        let (Some(recent), Some(entry), Some(popover)) =
            (recent.upgrade(), entry.upgrade(), popover.upgrade())
        else {
            return;
        };
        if servers.is_empty() || recent.first_child().is_some() {
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
            recent.append(&choice_button(&server, &server, &entry, &popover));
        }
    });
}

/// A line of the chooser reading `label`, which types `text`.
fn choice_button(label: &str, text: &str, entry: &gtk::Entry, popover: &gtk::Popover) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let name = gtk::Label::builder()
        .label(label)
        .hexpand(true)
        .xalign(0.0)
        .build();
    content.append(&name);
    if label != text {
        content.append(
            &gtk::Label::builder()
                .label(text)
                .css_classes(["dim-label"])
                .build(),
        );
    }
    let button = gtk::Button::builder()
        .child(&content)
        .focus_on_click(false)
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
