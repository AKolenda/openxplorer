// SPDX-License-Identifier: AGPL-3.0-only
//! "Set up network mount (SMB)" at the end of the Location tab (NET-027).
//!
//! Ports the `network-mount-assistant` part of `renderLocationPanel` in
//! `v2.0.0:desktop/ui/app.js` over ox-core's [`mount_plan`], which ports
//! `mount_plan` in `v2.0.0:desktop/mount_support.py`. The assistant only prepares
//! a command for the user to review and run in a terminal; it never runs
//! anything and never asks for an administrator itself.

use std::rc::Rc;

use gtk::prelude::*;
use ox_core::network::{mount_plan, DesktopUser, MountPlan};

use super::general_panel::glyph_button;
use crate::dialog::{labelled_entry, note, quiet_text};
use crate::icons::Icon;
use crate::window::BrowserWindow;

/// Why the assistant exists and what it does not do.
const INTRO: &str = "An SMB bookmark is not a permanent Linux path. This assistant prepares a command for \
                     an on-demand CIFS mount; it does not run it. Administrator approval is required in \
                     your terminal.";
/// The steps above a prepared command.
const STEPS: &str = "1. Install cifs-utils if needed. 2. Review and run the command in a terminal. 3. \
                     Return here and use the Linux path below.";
/// What the helper the command runs changes.
const HELPER_NOTE: &str = "The helper creates two systemd units and a root-only plaintext SMB credential \
                           file. It uses SMB 3.0, prompts before changes, does not edit fstab, and refuses \
                           to overwrite existing configuration. This system mount is separate from your \
                           GVfs/keyring session; “Sign out of server” does not remove it.";
/// The toast after Copy command.
pub(super) const COMMAND_COPIED: &str = "Setup command copied. Review it before running.";

/// The collapsed assistant. "Use this path" hands the planned Linux folder
/// to `use_path`, which fills Folder location.
pub(super) fn mount_assistant(use_path: impl Fn(&str) + 'static) -> gtk::Expander {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&quiet_text(INTRO));
    let address = labelled_entry(&content, "Network folder", "");
    address.set_placeholder_text(Some("\\\\archive-nas\\Shared"));
    let result = gtk::Box::new(gtk::Orientation::Vertical, 0);
    result.add_css_class("mount-plan");
    let prepare = glyph_button("Prepare setup command", Icon::Organization);
    prepare.set_halign(gtk::Align::Start);
    let use_path: Rc<dyn Fn(&str)> = Rc::new(use_path);
    prepare.connect_clicked(gtk::glib::clone!(
        #[weak]
        address,
        #[weak]
        result,
        move |_| show_plan(&result, &address.text(), Rc::clone(&use_path))
    ));
    content.append(&prepare);
    content.append(&result);
    gtk::Expander::builder()
        .label("Set up network mount (SMB)")
        .child(&content)
        .css_classes(["network-mount-assistant"])
        .build()
}

/// Replaces `result` with the plan for `address`, or the reason there is
/// none.
fn show_plan(result: &gtk::Box, address: &str, use_path: Rc<dyn Fn(&str)>) {
    while let Some(child) = result.first_child() {
        result.remove(&child);
    }
    match mount_plan(address, DesktopUser::current()) {
        Ok(plan) => fill_plan(result, &plan, use_path),
        Err(error) => result.append(&quiet_text(&error.to_string())),
    }
}

/// The steps, the command with Copy command, the Linux folder with Use
/// this path, the helper's note and the removal command.
fn fill_plan(result: &gtk::Box, plan: &MountPlan, use_path: Rc<dyn Fn(&str)>) {
    result.append(&quiet_text(STEPS));
    result.append(&command_view(&plan.command));
    let copy = glyph_button("Copy command", Icon::Copy);
    copy.set_halign(gtk::Align::Start);
    let command = plan.command.clone();
    copy.connect_clicked(move |button| {
        button.clipboard().set_text(&command);
        if let Some(window) = button.root().and_downcast::<BrowserWindow>() {
            window.show_message(COMMAND_COPIED);
        }
    });
    result.append(&copy);
    let target = plan.target_path.to_string_lossy().into_owned();
    let target_label = quiet_text(&format!("Linux folder: {target}"));
    target_label.set_selectable(true);
    target_label.add_css_class("mount-target");
    result.append(&target_label);
    let use_this_path = glyph_button("Use this path", Icon::Folder);
    use_this_path.set_halign(gtk::Align::Start);
    use_this_path.connect_clicked(move |_| use_path(&target));
    result.append(&use_this_path);
    result.append(&note(HELPER_NOTE));
    let removal = quiet_text(&plan.remove_command);
    removal.set_selectable(true);
    removal.add_css_class("monospace");
    let removal_section = gtk::Expander::builder()
        .label("Removal command (after restoring folder locations)")
        .child(&removal)
        .build();
    result.append(&removal_section);
}

/// The prepared command in a read-only text area (`.setup-command`).
fn command_view(command: &str) -> gtk::TextView {
    let view = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .css_classes(["setup-command"])
        .build();
    view.buffer().set_text(command);
    view.update_property(&[gtk::accessible::Property::Label("Mount setup command")]);
    view
}

#[cfg(test)]
mod tests;
