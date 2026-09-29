// SPDX-License-Identifier: AGPL-3.0-only
//! Changing an item's permissions on the Permissions tab (PROP-007), as
//! Dolphin's and Files' Permissions tabs do: the access of the owner, the
//! group and others, whether a file is executable, whether only owners
//! rename and delete a folder's content, and whether a folder's change
//! reaches everything inside it. Only the item's owner can change them;
//! previous versions stay read-only.

use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::permissions::{apply_in_background, Access, PermissionChange};
use ox_core::transfer::Cancellation;
use ox_core::versions::PreviousVersions;

use super::general_panel::glyph_button;
use crate::dialog_layer::{check_row, DialogFrame, PropertyGrid};
use crate::icons::Icon;

/// The toast after the permissions were changed.
const CHANGED: &str = "Permissions changed.";

/// The editor's controls.
#[derive(Debug, Clone)]
struct Controls {
    owner: gtk::DropDown,
    group: gtk::DropDown,
    others: gtk::DropDown,
    /// "Is executable" for a file, "Only owner can rename and delete
    /// folder content" for a folder.
    special: gtk::CheckButton,
    /// "Apply changes to all subfolders and their contents", for a folder.
    recursive: Option<gtk::CheckButton>,
}

impl Controls {
    /// The change the controls describe.
    fn change(&self, is_folder: bool) -> PermissionChange {
        let access = |choice: &gtk::DropDown| {
            let index = usize::try_from(choice.selected()).unwrap_or_default();
            Access::ALL.get(index).copied().unwrap_or(Access::None)
        };
        let special = self.special.is_active();
        PermissionChange {
            owner: access(&self.owner),
            group: access(&self.group),
            others: access(&self.others),
            executable: !is_folder && special,
            owners_only_delete: is_folder && special,
        }
    }
}

/// A choice of the three accesses, showing `current`.
fn access_choice(label: &str, current: Access) -> gtk::DropDown {
    let labels: Vec<&str> = Access::ALL.iter().map(|access| access.label()).collect();
    let choice = gtk::DropDown::from_strings(&labels);
    let index = Access::ALL.iter().position(|access| *access == current).unwrap_or_default();
    choice.set_selected(u32::try_from(index).unwrap_or_default());
    choice.set_halign(gtk::Align::Start);
    choice.update_property(&[gtk::accessible::Property::Label(label)]);
    choice
}

/// The editor for the item at `uri`, whose permission bits are `mode`.
pub(super) fn permissions_editor(
    uri: &str,
    mode: u32,
    is_folder: bool,
    versions: Arc<PreviousVersions>,
) -> gtk::Box {
    let editor = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let current = PermissionChange::of_mode(mode, is_folder);
    let grid = PropertyGrid::new();
    let owner = access_choice("Owner access", current.owner);
    let group = access_choice("Group access", current.group);
    let others = access_choice("Others access", current.others);
    for (line, (name, choice)) in (0..).zip([("Owner", &owner), ("Group", &group), ("Others", &others)]) {
        grid.add_row(&format!("{name} access"), "");
        // The choice takes the place of the empty value.
        if let Some(value) = grid.widget().child_at(1, line) {
            grid.widget().remove(&value);
        }
        grid.widget().attach(choice, 1, line, 1, 1);
    }
    editor.append(grid.widget());
    let special = if is_folder {
        check_row("Only owner can rename and delete folder content", current.owners_only_delete)
    } else {
        check_row("Is executable", current.executable)
    };
    editor.append(&special);
    let recursive = is_folder.then(|| check_row("Apply changes to all subfolders and their contents", false));
    if let Some(recursive) = &recursive {
        editor.append(recursive);
    }
    let controls = Controls {
        owner,
        group,
        others,
        special,
        recursive,
    };
    let apply = glyph_button("Apply permissions", Icon::ShieldLock);
    apply.set_halign(gtk::Align::Start);
    let uri = uri.to_owned();
    apply.connect_clicked(move |button| {
        let change = controls.change(is_folder);
        let recursive = controls.recursive.as_ref().is_some_and(gtk::CheckButton::is_active);
        start_change(button, &uri, change, recursive, &versions);
    });
    editor.append(&apply);
    editor
}

/// Applies `change` in the background and says how it went: a toast, or
/// the error in the dialog.
fn start_change(
    button: &gtk::Button,
    uri: &str,
    change: PermissionChange,
    recursive: bool,
    versions: &PreviousVersions,
) {
    let frame = button
        .ancestor(DialogFrame::static_type())
        .and_downcast::<DialogFrame>();
    let window = button.root().and_downcast::<crate::window::BrowserWindow>();
    // Safety rule PROP-024: a previous version is never changed in place.
    if let Err(refusal) = versions.check_writable(uri) {
        if let Some(frame) = frame {
            frame.show_error(&refusal.to_string());
        }
        return;
    }
    button.set_sensitive(false);
    let changing = apply_in_background(uri.to_owned(), change, recursive, Cancellation::new());
    let button = button.downgrade();
    glib::spawn_future_local(async move {
        let result = changing.await;
        if let Some(button) = button.upgrade() {
            button.set_sensitive(true);
        }
        match (result, frame, window) {
            (Ok(()), _, Some(window)) => window.show_message(CHANGED),
            (Err(error), Some(frame), _) => frame.show_error(&error.to_string()),
            _ => {}
        }
    });
}
