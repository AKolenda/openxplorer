// SPDX-License-Identifier: AGPL-3.0-only
//! Changing permissions on the Permissions tab (PROP-007), as Dolphin's
//! and Files' Permissions tabs do: the access of the owner, the group and
//! others, whether a file is executable, whether only owners rename and
//! delete a folder's content, whether a folder's change reaches everything
//! inside it, the group (and, for the superuser, the owner), and Dolphin's
//! Advanced Permissions. Only the items' owner can change them; previous
//! versions stay read-only. With several items selected, the change
//! applies to all of them (PROP-002).

mod advanced;

use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::permissions::{
    apply_in_background, current_user_is_superuser, group_choices, user_choices, Access, Account, ModeChange,
    PermissionChange, PermissionRequest,
};
use ox_core::transfer::Cancellation;
use ox_core::versions::PreviousVersions;

use super::general_panel::glyph_button;
use crate::dialog_layer::{check_row, DialogFrame, PropertyGrid};
use crate::icons::Icon;
use advanced::AdvancedBits;

/// The toast after the permissions were changed.
const CHANGED: &str = "Permissions changed.";

/// The items the editor changes, as the tab first shows them.
#[derive(Debug, Clone)]
pub(super) struct EditedItems {
    /// Every item changed.
    pub uris: Vec<String>,
    /// The permission bits of the first item, which the controls show.
    pub mode: u32,
    /// Some item is a file.
    pub has_files: bool,
    /// Some item is a folder.
    pub has_folders: bool,
    /// The first item's owner.
    pub owner: Option<Account>,
    /// The first item's group.
    pub group: Option<Account>,
}

/// A choice of accounts, showing the current one.
#[derive(Debug, Clone)]
struct AccountChoice {
    choice: gtk::DropDown,
    accounts: Vec<Account>,
    current: Option<u32>,
}

impl AccountChoice {
    /// `accounts`, with `current` first when it is not among them.
    fn new(label: &str, current: Option<&Account>, mut accounts: Vec<Account>) -> Self {
        if let Some(current) = current.filter(|current| !accounts.contains(current)) {
            accounts.insert(0, current.clone());
        }
        let names: Vec<&str> = accounts.iter().map(|account| account.name.as_str()).collect();
        let choice = gtk::DropDown::from_strings(&names);
        let index = current
            .and_then(|current| accounts.iter().position(|account| account.id == current.id))
            .unwrap_or_default();
        choice.set_selected(u32::try_from(index).unwrap_or_default());
        choice.set_halign(gtk::Align::Start);
        choice.update_property(&[gtk::accessible::Property::Label(label)]);
        Self {
            choice,
            accounts,
            current: current.map(|account| account.id),
        }
    }

    /// The chosen id, when it differs from the current one.
    fn changed(&self) -> Option<u32> {
        let index = usize::try_from(self.choice.selected()).ok()?;
        let chosen = self.accounts.get(index)?.id;
        (Some(chosen) != self.current).then_some(chosen)
    }
}

/// The editor's controls.
#[derive(Debug, Clone)]
struct Controls {
    owner: gtk::DropDown,
    group: gtk::DropDown,
    others: gtk::DropDown,
    /// "Is executable", when a file is edited.
    executable: Option<gtk::CheckButton>,
    /// "Only owner can rename and delete folder content", when a folder
    /// is edited.
    owners_only_delete: Option<gtk::CheckButton>,
    /// "Apply changes to all subfolders and their contents", when a
    /// folder is edited.
    recursive: Option<gtk::CheckButton>,
    /// The item's group, and its owner for the superuser.
    group_account: AccountChoice,
    owner_account: Option<AccountChoice>,
    /// Dolphin's Advanced Permissions.
    advanced: AdvancedBits,
}

impl Controls {
    /// The change the simple controls describe.
    fn simple_change(&self) -> PermissionChange {
        let access = |choice: &gtk::DropDown| {
            let index = usize::try_from(choice.selected()).unwrap_or_default();
            Access::ALL.get(index).copied().unwrap_or(Access::None)
        };
        PermissionChange {
            owner: access(&self.owner),
            group: access(&self.group),
            others: access(&self.others),
            executable: is_checked(self.executable.as_ref()),
            owners_only_delete: is_checked(self.owners_only_delete.as_ref()),
        }
    }

    /// The request the controls describe.
    fn request(&self) -> PermissionRequest {
        let mode = if self.advanced.is_expanded() {
            ModeChange::Advanced(self.advanced.mode())
        } else {
            ModeChange::Simple(self.simple_change())
        };
        PermissionRequest {
            mode,
            owner: self.owner_account.as_ref().and_then(AccountChoice::changed),
            group: self.group_account.changed(),
            recursive: is_checked(self.recursive.as_ref()),
        }
    }

    /// The simple controls, which the advanced ones replace while open.
    fn simple_controls(&self) -> Vec<gtk::Widget> {
        let mut controls = vec![
            self.owner.clone().upcast(),
            self.group.clone().upcast(),
            self.others.clone().upcast(),
        ];
        controls.extend(self.executable.iter().map(|check| check.clone().upcast()));
        controls.extend(self.owners_only_delete.iter().map(|check| check.clone().upcast()));
        controls
    }
}

/// Whether `check` exists and is ticked.
fn is_checked(check: Option<&gtk::CheckButton>) -> bool {
    check.is_some_and(gtk::CheckButton::is_active)
}

/// A choice of the three accesses, showing `current`.
fn access_choice(label: &str, current: Access) -> gtk::DropDown {
    let labels: Vec<&str> = Access::ALL.iter().map(|access| access.label()).collect();
    let choice = gtk::DropDown::from_strings(&labels);
    let index = Access::ALL
        .iter()
        .position(|access| *access == current)
        .unwrap_or_default();
    choice.set_selected(u32::try_from(index).unwrap_or_default());
    choice.set_halign(gtk::Align::Start);
    choice.update_property(&[gtk::accessible::Property::Label(label)]);
    choice
}

/// Adds the row `name` holding `control` to `grid`.
fn add_control_row(grid: &PropertyGrid, name: &str, control: &impl IsA<gtk::Widget>) {
    let value = grid.add_row(name, "");
    // The control takes the place of the empty value.
    let (column, line, _, _) = grid.widget().query_child(&value);
    grid.widget().remove(&value);
    grid.widget().attach(control, column, line, 1, 1);
}

/// The editor for `items`.
pub(super) fn permissions_editor(items: EditedItems, versions: Arc<PreviousVersions>) -> gtk::Box {
    let editor = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let only_folders = items.has_folders && !items.has_files;
    let current = PermissionChange::of_mode(items.mode, only_folders);
    let grid = PropertyGrid::new();
    let owner = access_choice("Owner access", current.owner);
    let group = access_choice("Group access", current.group);
    let others = access_choice("Others access", current.others);
    for (name, choice) in [("Owner", &owner), ("Group", &group), ("Others", &others)] {
        add_control_row(&grid, &format!("{name} access"), choice);
    }
    let owner_account = current_user_is_superuser().then(|| {
        let choice = AccountChoice::new("Owner", items.owner.as_ref(), user_choices());
        add_control_row(&grid, "Owner", &choice.choice);
        choice
    });
    let group_account = AccountChoice::new("Group", items.group.as_ref(), group_choices());
    add_control_row(&grid, "Group", &group_account.choice);
    editor.append(grid.widget());
    let executable = items
        .has_files
        .then(|| check_row("Is executable", items.mode & 0o100 != 0));
    let owners_only_delete = items.has_folders.then(|| {
        check_row(
            "Only owner can rename and delete folder content",
            items.mode & 0o1000 != 0,
        )
    });
    for check in executable.iter().chain(&owners_only_delete) {
        editor.append(check);
    }
    let recursive = items
        .has_folders
        .then(|| check_row("Apply changes to all subfolders and their contents", false));
    let advanced = AdvancedBits::new(items.mode);
    editor.append(advanced.widget());
    if let Some(recursive) = &recursive {
        editor.append(recursive);
    }
    let controls = Controls {
        owner,
        group,
        others,
        executable,
        owners_only_delete,
        recursive,
        group_account,
        owner_account,
        advanced,
    };
    // Opening the advanced controls starts them from what the simple
    // ones say; while open, they replace them.
    let simple = controls.clone();
    let (mode, only_folders) = (items.mode, only_folders);
    controls.advanced.connect_expanded(move |expanded| {
        if expanded {
            let bits = simple.simple_change().apply_to(mode, only_folders);
            simple.advanced.show(bits);
        }
        for control in simple.simple_controls() {
            control.set_sensitive(!expanded);
        }
    });
    let apply = glyph_button("Apply permissions", Icon::ShieldLock);
    apply.set_halign(gtk::Align::Start);
    apply.connect_clicked(move |button| {
        start_change(button, &items.uris, controls.request(), &versions);
    });
    editor.append(&apply);
    editor
}

/// Applies `request` to every item in the background and says how it
/// went: a toast, or the first error in the dialog.
fn start_change(
    button: &gtk::Button,
    uris: &[String],
    request: PermissionRequest,
    versions: &PreviousVersions,
) {
    let frame = button
        .ancestor(DialogFrame::static_type())
        .and_downcast::<DialogFrame>();
    let window = button.root().and_downcast::<crate::window::BrowserWindow>();
    // Safety rule PROP-024: a previous version is never changed in place.
    if let Some(refusal) = uris.iter().find_map(|uri| versions.check_writable(uri).err()) {
        if let Some(frame) = frame {
            frame.show_error(&refusal.to_string());
        }
        return;
    }
    button.set_sensitive(false);
    let uris = uris.to_vec();
    let button = button.downgrade();
    glib::spawn_future_local(async move {
        let mut result = Ok(());
        for uri in uris {
            result = apply_in_background(uri, request, Cancellation::new()).await;
            if result.is_err() {
                break;
            }
        }
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
