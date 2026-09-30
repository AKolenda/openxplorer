// SPDX-License-Identifier: AGPL-3.0-only
//! Changing permissions on the Permissions tab (PROP-007), as Dolphin's
//! and Files' Permissions tabs do: the access of the owner, the group and
//! others, whether a file is executable, whether only owners rename and
//! delete a folder's content, whether a folder's change reaches everything
//! inside it, the group (and, for the superuser, the owner), and Dolphin's
//! Advanced Permissions. Only the items' owner can change them; previous
//! versions stay read-only. With several items selected, what the user
//! changes applies to all of them and every other bit stays as each item
//! has it (PROP-002).

mod advanced;
mod choices;

use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::permissions::{
    apply_in_background, current_user_is_superuser, group_choices, user_choices, Access, Account, ModeChange,
    PermissionChange, PermissionClass, PermissionRequest,
};
use ox_core::transfer::Cancellation;
use ox_core::versions::PreviousVersions;

use super::general_panel::glyph_button;
use crate::dialog_layer::{check_row, DialogFrame, PropertyGrid};
use crate::icons::Icon;
use advanced::AdvancedBits;
use choices::{AccessChoice, AccountChoice, StateCheck};

/// The toast after the permissions were changed.
const CHANGED: &str = "Permissions changed.";

/// An item the editor changes.
#[derive(Debug, Clone)]
pub(super) struct EditedItem {
    pub uri: String,
    /// Its permission bits.
    pub mode: u32,
    pub is_folder: bool,
}

/// The items the editor changes, as the tab first shows them.
#[derive(Debug, Clone)]
pub(super) struct EditedItems {
    /// Every item changed.
    pub items: Vec<EditedItem>,
    /// The items' owner, when they share one.
    pub owner: Option<Account>,
    /// The items' group, when they share one.
    pub group: Option<Account>,
}

impl EditedItems {
    /// The value `value` gives every item, if they share one; items for
    /// which it gives `None` do not count.
    fn common<T: PartialEq>(&self, value: impl Fn(&EditedItem) -> Option<T>) -> Option<T> {
        let mut values = self.items.iter().filter_map(value);
        let first = values.next()?;
        values.all(|other| other == first).then_some(first)
    }

    /// The mode every item has, if they share one.
    fn common_mode(&self) -> Option<u32> {
        self.common(|item| Some(item.mode))
    }
}

/// The editor's controls.
#[derive(Debug, Clone)]
struct Controls {
    owner: AccessChoice,
    group: AccessChoice,
    others: AccessChoice,
    /// "Is executable", when a file is edited.
    executable: Option<StateCheck>,
    /// "Only owner can rename and delete folder content", when a folder
    /// is edited.
    owners_only_delete: Option<StateCheck>,
    /// "Apply changes to all subfolders and their contents", when a
    /// folder is edited.
    recursive: Option<gtk::CheckButton>,
    /// The items' group, and their owner for the superuser.
    group_account: AccountChoice,
    owner_account: Option<AccountChoice>,
    /// Dolphin's Advanced Permissions, when every item has the same mode.
    advanced: Option<(AdvancedBits, u32)>,
}

impl Controls {
    /// The change the simple controls describe.
    fn simple_change(&self) -> PermissionChange {
        let recursive = self.is_recursive();
        PermissionChange {
            owner: self.owner.change(recursive),
            group: self.group.change(recursive),
            others: self.others.change(recursive),
            executable: self.executable.as_ref().and_then(StateCheck::change),
            owners_only_delete: self.owners_only_delete.as_ref().and_then(StateCheck::change),
        }
    }

    fn is_recursive(&self) -> bool {
        self.recursive.as_ref().is_some_and(gtk::CheckButton::is_active)
    }

    /// The open Advanced Permissions and the mode they started from.
    fn open_advanced(&self) -> Option<(&AdvancedBits, u32)> {
        self.advanced
            .as_ref()
            .filter(|(advanced, _)| advanced.is_expanded())
            .map(|(advanced, mode)| (advanced, *mode))
    }

    /// The request the controls describe.
    fn request(&self) -> PermissionRequest {
        let mode = match self.open_advanced() {
            Some((advanced, _)) => ModeChange::Advanced(advanced.mode()),
            None => ModeChange::Simple(self.simple_change()),
        };
        PermissionRequest {
            mode,
            owner: self.owner_account.as_ref().and_then(AccountChoice::changed),
            group: self.group_account.changed(),
            recursive: self.is_recursive(),
        }
    }

    /// Whether applying would change anything.
    fn has_changes(&self) -> bool {
        let request = self.request();
        let mode_changes = match (request.mode, self.open_advanced()) {
            (ModeChange::Advanced(bits), Some((_, mode))) => bits != mode || request.recursive,
            (ModeChange::Simple(change), _) => !change.is_empty(),
            (ModeChange::Advanced(_), None) => true,
        };
        mode_changes || request.owner.is_some() || request.group.is_some()
    }

    /// The simple controls, which the advanced ones replace while open.
    fn simple_controls(&self) -> Vec<gtk::Widget> {
        let mut controls = vec![
            self.owner.choice.clone().upcast(),
            self.group.choice.clone().upcast(),
            self.others.choice.clone().upcast(),
        ];
        controls.extend(self.executable.iter().map(|check| check.check.clone().upcast()));
        controls.extend(
            self.owners_only_delete
                .iter()
                .map(|check| check.check.clone().upcast()),
        );
        controls
    }

    /// Runs `changed` whenever a control changes.
    fn connect_changed(&self, changed: impl Fn() + Clone + 'static) {
        let mut choices = vec![&self.owner.choice, &self.group.choice, &self.others.choice];
        choices.push(&self.group_account.choice);
        choices.extend(self.owner_account.as_ref().map(|account| &account.choice));
        for choice in choices {
            let changed = changed.clone();
            choice.connect_selected_notify(move |_| changed());
        }
        let checks = self.executable.iter().chain(&self.owners_only_delete);
        for check in checks.map(|check| &check.check).chain(&self.recursive) {
            let changed = changed.clone();
            check.connect_toggled(move |_| changed());
        }
        if let Some((advanced, _)) = &self.advanced {
            advanced.connect_changed(changed);
        }
    }
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
    let has_files = items.items.iter().any(|item| !item.is_folder);
    let has_folders = items.items.iter().any(|item| item.is_folder);
    let grid = PropertyGrid::new();
    let access = |name: &str, class: PermissionClass| {
        let label = format!("{name} access");
        let choice = AccessChoice::new(&label, items.common(|item| Some(Access::of(item.mode, class))));
        add_control_row(&grid, &label, &choice.choice);
        choice
    };
    let owner = access("Owner", PermissionClass::Owner);
    let group = access("Group", PermissionClass::Group);
    let others = access("Others", PermissionClass::Others);
    let owner_account = current_user_is_superuser().then(|| {
        let choice = AccountChoice::new("Owner", items.owner.as_ref(), user_choices());
        add_control_row(&grid, "Owner", &choice.choice);
        choice
    });
    let group_account = AccountChoice::new("Group", items.group.as_ref(), group_choices());
    add_control_row(&grid, "Group", &group_account.choice);
    editor.append(grid.widget());
    let executable = has_files.then(|| {
        let is_executable = items.common(|item| (!item.is_folder).then_some(item.mode & 0o100 != 0));
        StateCheck::new("Is executable", is_executable)
    });
    let owners_only_delete = has_folders.then(|| {
        let is_sticky = items.common(|item| item.is_folder.then_some(item.mode & 0o1000 != 0));
        StateCheck::new("Only owner can rename and delete folder content", is_sticky)
    });
    for check in executable.iter().chain(&owners_only_delete) {
        editor.append(&check.check);
    }
    let recursive =
        has_folders.then(|| check_row("Apply changes to all subfolders and their contents", false));
    let advanced = items.common_mode().map(|mode| {
        let advanced = AdvancedBits::new(mode);
        editor.append(advanced.widget());
        (advanced, mode)
    });
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
    if let Some((advanced, mode)) = &controls.advanced {
        let simple = controls.clone();
        let (advanced, mode) = (advanced.clone(), *mode);
        let only_folders = has_folders && !has_files;
        advanced.clone().connect_expanded(move |expanded| {
            if expanded {
                advanced.show(simple.simple_change().apply_to(mode, only_folders));
            }
            for control in simple.simple_controls() {
                control.set_sensitive(!expanded);
            }
        });
    }
    let apply = glyph_button("Apply permissions", Icon::ShieldLock);
    apply.set_halign(gtk::Align::Start);
    // Nothing is applied until the user changes something.
    apply.set_sensitive(false);
    let watched = controls.clone();
    let button = apply.downgrade();
    controls.connect_changed(move || {
        if let Some(button) = button.upgrade() {
            button.set_sensitive(watched.has_changes());
        }
    });
    let uris: Vec<String> = items.items.into_iter().map(|item| item.uri).collect();
    apply.connect_clicked(move |button| {
        start_change(button, &uris, controls.request(), &versions);
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
