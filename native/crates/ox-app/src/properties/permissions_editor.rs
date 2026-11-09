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
pub(super) mod choices;

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::permissions::{
    apply_in_background, current_user_is_superuser, group_choices, user_choices, Access, Account, ModeChange,
    PermissionChange, PermissionClass, PermissionRequest,
};
use ox_core::transfer::Cancellation;
use ox_core::versions::PreviousVersions;

use super::window::glyph_button;
use super::window::{ApplyFuture, Pending, PropertiesWindow};
use crate::dialog::{check_row, PropertyGrid};
use crate::icons::Icon;
use advanced::AdvancedBits;
pub(super) use choices::AccountChoice as SharedAccountChoice;
use choices::{AccessChoice, AccountChoice, StateCheck};

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
            (ModeChange::Advanced(_), None) | (ModeChange::ReadOnlyFiles(_) | ModeChange::Bits { .. }, _) => {
                true
            }
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

/// The editor for `items`, and its change for the window's OK and Apply.
pub(super) fn permissions_editor(
    items: EditedItems,
    versions: Arc<PreviousVersions>,
) -> (gtk::Box, Rc<EditorChange>) {
    let editor = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let has_files = items.items.iter().any(|item| !item.is_folder);
    let has_folders = items.items.iter().any(|item| item.is_folder);
    let grid = PropertyGrid::new();
    let access = |name: &str, class: PermissionClass| {
        let label = ox_core::i18n::format_message("{name} access", &[("name", name)]);
        let choice = AccessChoice::new(&label, items.common(|item| Some(Access::of(item.mode, class))));
        add_control_row(&grid, &label, &choice.choice);
        choice
    };
    let owner = access(ox_core::i18n::gettext_static("Owner"), PermissionClass::Owner);
    let group = access(ox_core::i18n::gettext_static("Group"), PermissionClass::Group);
    let others = access(ox_core::i18n::gettext_static("Others"), PermissionClass::Others);
    let owner_account = current_user_is_superuser().then(|| {
        let choice = AccountChoice::new(
            ox_core::i18n::gettext_static("Owner"),
            items.owner.as_ref(),
            user_choices(),
        );
        add_control_row(&grid, &ox_core::i18n::gettext("Owner"), &choice.choice);
        choice
    });
    let group_account = AccountChoice::new(
        ox_core::i18n::gettext_static("Group"),
        items.group.as_ref(),
        group_choices(),
    );
    add_control_row(&grid, &ox_core::i18n::gettext("Group"), &group_account.choice);
    editor.append(grid.widget());
    let executable = has_files.then(|| {
        let is_executable = items.common(|item| (!item.is_folder).then_some(item.mode & 0o100 != 0));
        StateCheck::new(ox_core::i18n::gettext_static("Is executable"), is_executable)
    });
    let owners_only_delete = has_folders.then(|| {
        let is_sticky = items.common(|item| item.is_folder.then_some(item.mode & 0o1000 != 0));
        StateCheck::new(
            ox_core::i18n::gettext_static("Only owner can rename and delete folder content"),
            is_sticky,
        )
    });
    for check in executable.iter().chain(&owners_only_delete) {
        editor.append(&check.check);
    }
    let recursive = has_folders.then(|| {
        check_row(
            &ox_core::i18n::gettext("Apply changes to all subfolders and their contents"),
            false,
        )
    });
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
    let uris = items.items.into_iter().map(|item| item.uri).collect();
    let (apply, change) = apply_button(&controls, uris, versions);
    editor.append(&apply);
    (editor, change)
}

/// Apply permissions, greyed until the user changes something, and the
/// change it makes, which the window's OK and Apply make too.
fn apply_button(
    controls: &Controls,
    uris: Vec<String>,
    versions: Arc<PreviousVersions>,
) -> (gtk::Button, Rc<EditorChange>) {
    let apply = glyph_button(
        ox_core::i18n::gettext_static("Apply permissions"),
        Icon::ShieldLock,
    );
    apply.set_halign(gtk::Align::Start);
    apply.set_sensitive(false);
    let change = Rc::new(EditorChange {
        controls: controls.clone(),
        uris: RefCell::new(uris),
        applied: Cell::new(None),
        rereads: RefCell::new(Vec::new()),
        versions,
        button: apply.downgrade(),
    });
    let watched = Rc::clone(&change);
    controls.connect_changed(move || watched.changed());
    let pressed = Rc::clone(&change);
    apply.connect_clicked(move |button| {
        let change = Rc::clone(&pressed);
        let window = PropertiesWindow::of(button);
        glib::spawn_future_local(async move {
            if let (Err(message), Some(window)) = (change.apply_now().await, window) {
                window.show_error(&message);
            }
        });
    });
    (apply, change)
}

/// Reads the items again after the editor changed them.
type Reread = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()>>>>;

/// The editor's change, which Apply permissions and the window's OK and
/// Apply make (PROP-002).
pub(super) struct EditorChange {
    controls: Controls,
    /// The items' locations, which Hidden on General changes.
    uris: RefCell<Vec<String>>,
    /// The request last made, which the window's next OK or Apply does not
    /// make again over a Read-only applied after it.
    applied: Cell<Option<PermissionRequest>>,
    /// Run once each change is made, before the window's next one.
    rereads: RefCell<Vec<Reread>>,
    versions: Arc<PreviousVersions>,
    /// Apply permissions, greyed while there is nothing to apply.
    button: glib::WeakRef<gtk::Button>,
}

impl EditorChange {
    /// Whether the controls hold a change not made yet.
    fn has_changes(&self) -> bool {
        self.controls.has_changes() && self.applied.get() != Some(self.controls.request())
    }

    /// Greys Apply permissions, and the window's Apply, or not.
    fn changed(&self) {
        let has_changes = self.has_changes();
        if let Some(button) = self.button.upgrade() {
            button.set_sensitive(has_changes);
            if let Some(window) = PropertiesWindow::of(&button) {
                window.changed();
            }
        }
    }

    /// Points the change at `to` for the item that was at `from`, once the
    /// General tab renamed it.
    pub(super) fn follow_rename(&self, from: &str, to: &str) {
        for uri in self.uris.borrow_mut().iter_mut().filter(|uri| *uri == from) {
            to.clone_into(uri);
        }
    }

    /// Runs `reread` each time the editor changed the items, even when it
    /// stopped at an error, and waits for it before Apply permissions
    /// comes back or the window's next change is made.
    pub(super) fn connect_applied<F: Future<Output = ()> + 'static>(&self, reread: impl Fn() -> F + 'static) {
        self.rereads
            .borrow_mut()
            .push(Box::new(move || Box::pin(reread())));
    }

    /// Applies the controls' change to every item in the background.
    ///
    /// # Errors
    ///
    /// The refusal or the first error, to show in a message.
    async fn apply_now(&self) -> Result<(), String> {
        // Safety rule PROP-024: a previous version is never changed in place.
        let uris = self.uris.borrow().clone();
        if let Some(refusal) = uris
            .iter()
            .find_map(|uri| self.versions.check_writable(uri).err())
        {
            return Err(refusal.to_string());
        }
        let button = self.button.upgrade();
        let opener = button
            .as_ref()
            .and_then(PropertiesWindow::of)
            .and_then(|window| window.opener());
        // No drive is formatted while the items change, nor are they
        // changed while one is (DEV-012).
        let _writing = match &opener {
            Some(opener) => Some(opener.hold_write()?),
            None => None,
        };
        if let Some(button) = &button {
            button.set_sensitive(false);
        }
        let request = self.controls.request();
        let mut result = Ok(());
        self.applied.set(None);
        for uri in uris {
            result = apply_in_background(uri, request, Cancellation::new()).await;
            if result.is_err() {
                break;
            }
        }
        if result.is_ok() {
            self.applied.set(Some(request));
        }
        let rereads: Vec<_> = self.rereads.borrow().iter().map(|reread| reread()).collect();
        for reread in rereads {
            reread.await;
        }
        self.changed();
        result.map_err(|error| error.to_string())
    }
}

impl Pending for EditorChange {
    fn has_changes(&self) -> bool {
        EditorChange::has_changes(self)
    }

    fn apply(self: Rc<Self>) -> ApplyFuture {
        Box::pin(async move { self.apply_now().await })
    }
}
