// SPDX-License-Identifier: AGPL-3.0-only
//! The Attributes row of several selected items (PROP-002), as Windows 11
//! shows it: Read-only and Hidden, each ticked when every item has it,
//! clear when none has, and half ticked when they differ. With a folder
//! among them, Read-only reads "Read-only (Only applies to files in
//! folder)" and starts half ticked, as one folder's does on the General
//! tab ([`super::general_tab`]). A half-ticked box moves on through
//! ticked, clear and unchanged with each click, as Windows' does.
//!
//! OK or Apply makes the change to every item, after the Security tab's:
//! Read-only takes every write permission from the files (inside a
//! folder) or gives their owner write again, and Hidden adds or removes
//! each name's leading dot, which is how Linux hides an item, with the
//! normal refusal when that name is taken. The boxes stay greyed until
//! every item is read, and each stays greyed unless every item may be
//! changed that way.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::Entry;
use ox_core::permissions::{apply_outside_in_background, ModeChange, PermissionRequest};
use ox_core::transfer::Cancellation;
use ox_core::versions::PreviousVersions;

use super::metadata::ItemProperties;
use super::selection_view::selection_title;
use super::window::{ApplyFuture, Pending, PropertiesWindow};
use crate::window::BrowserWindow;

/// A check box over items that may differ.
struct MixedCheck {
    check: gtk::CheckButton,
    /// What every item has now; `None` while they differ or are not read.
    now: Cell<Option<bool>>,
    /// What the user chose; `None` for no change.
    wanted: Cell<Option<bool>>,
    /// Set while the box is changed here rather than by the user.
    is_updating: Cell<bool>,
}

impl MixedCheck {
    fn new(label: &str, now: Option<bool>) -> Rc<Self> {
        let mixed = Rc::new(Self {
            check: gtk::CheckButton::with_mnemonic(label),
            now: Cell::new(now),
            wanted: Cell::new(now),
            is_updating: Cell::new(false),
        });
        super::window::wrap_check_label(&mixed.check);
        mixed.check.set_sensitive(false);
        mixed.show();
        let weak = Rc::downgrade(&mixed);
        mixed.check.connect_toggled(move |_| {
            if let Some(mixed) = weak.upgrade() {
                mixed.toggled();
            }
        });
        mixed
    }

    /// Shows what every item has now, `now`, dropping any choice.
    fn reset(&self, now: Option<bool>) {
        self.now.set(now);
        self.wanted.set(now);
        self.show();
    }

    fn show(&self) {
        let shown = self.wanted.get();
        self.is_updating.set(true);
        self.check.set_inconsistent(shown.is_none());
        self.check.set_active(shown == Some(true));
        self.is_updating.set(false);
    }

    /// A box that started half ticked moves on to its next state; another
    /// one is ticked or cleared.
    fn toggled(&self) {
        if self.is_updating.get() {
            return;
        }
        if self.now.get().is_none() {
            let next = match self.wanted.get() {
                None => Some(true),
                Some(true) => Some(false),
                Some(false) => None,
            };
            self.wanted.set(next);
            self.show();
        } else {
            self.wanted.set(Some(self.check.is_active()));
        }
    }

    /// The state to give every item, if the user changed it.
    fn change(&self) -> Option<bool> {
        self.wanted.get().filter(|wanted| Some(*wanted) != self.now.get())
    }
}

/// One selected item, as the last OK or Apply left it.
#[derive(Debug, Clone)]
struct Item {
    uri: String,
    name: String,
    is_folder: bool,
    /// Renaming asks about a changed extension, except for a link to a
    /// folder, whose name has none (OPS-049).
    links_to_folder: bool,
}

/// What every one of several items has: `Some` of the state they share,
/// `None` when they differ.
fn shared(states: impl IntoIterator<Item = bool>) -> Option<bool> {
    let mut states = states.into_iter();
    let first = states.next()?;
    states.all(|state| state == first).then_some(first)
}

/// Told the old and new location of an item Hidden renamed.
type RenameFollower = Box<dyn Fn(&str, &str)>;

/// Read-only and Hidden of several items, until OK or Apply.
pub(super) struct SelectionAttributes {
    window: glib::WeakRef<PropertiesWindow>,
    /// The items, once read.
    items: RefCell<Vec<Item>>,
    read_only: Rc<MixedCheck>,
    hidden: Rc<MixedCheck>,
    /// Whether a folder is among the items, whose Read-only is never known.
    has_folder: bool,
    /// Which locations are previous versions, which are never changed.
    versions: Arc<PreviousVersions>,
    /// Told the old and new location of each item Hidden renames.
    renamed: RefCell<Vec<RenameFollower>>,
}

impl SelectionAttributes {
    /// The boxes for `entries`, greyed until [`Self::fill`].
    pub(super) fn new(
        window: &PropertiesWindow,
        entries: &[Entry],
        versions: Arc<PreviousVersions>,
    ) -> Rc<Self> {
        let has_folder = entries.iter().any(|entry| entry.is_dir);
        let read_only_label = if has_folder {
            ox_core::i18n::gettext("Read-only (Only applies to files in folder)")
        } else {
            ox_core::i18n::gettext("Read-only")
        };
        let hidden = shared(entries.iter().map(|entry| entry.name.starts_with('.')));
        let attributes = Rc::new(Self {
            window: window.downgrade(),
            items: RefCell::new(Vec::new()),
            read_only: MixedCheck::new(&read_only_label, None),
            hidden: MixedCheck::new(&ox_core::i18n::gettext("Hidden"), hidden),
            has_folder,
            versions,
            renamed: RefCell::new(Vec::new()),
        });
        for check in [&attributes.read_only.check, &attributes.hidden.check] {
            let window = window.downgrade();
            check.connect_toggled(move |_| {
                if let Some(window) = window.upgrade() {
                    window.changed();
                }
            });
        }
        attributes
    }

    /// Read-only and Hidden: side by side for files, one above the other
    /// when a folder's long Read-only label is shown.
    pub(super) fn widget(&self) -> gtk::Box {
        let orientation = if self.has_folder {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        };
        let boxes = gtk::Box::builder()
            .orientation(orientation)
            .css_classes(["properties-attributes"])
            .build();
        boxes.append(&self.read_only.check);
        boxes.append(&self.hidden.check);
        boxes
    }

    /// Shows the items as read, `items`, and lets Read-only and Hidden be
    /// changed when `can_change_permissions` and `can_rename` say so.
    pub(super) fn fill(&self, items: &[ItemProperties], can_change_permissions: bool, can_rename: bool) {
        self.items.replace(
            items
                .iter()
                .map(|item| Item {
                    uri: item.entry.uri.clone(),
                    name: item.entry.name.clone(),
                    is_folder: item.entry.is_dir,
                    links_to_folder: item.links_to_folder,
                })
                .collect(),
        );
        let read_only = shared(
            items
                .iter()
                .map(|item| item.mode.is_some_and(|mode| mode & 0o200 == 0)),
        );
        self.read_only.reset(read_only.filter(|_| !self.has_folder));
        self.hidden
            .reset(shared(items.iter().map(|item| item.entry.name.starts_with('.'))));
        self.read_only.check.set_sensitive(can_change_permissions);
        self.hidden.check.set_sensitive(can_rename);
    }

    /// Runs `renamed` with the old and new location of each item Hidden
    /// renames.
    pub(super) fn connect_renamed(&self, renamed: impl Fn(&str, &str) + 'static) {
        self.renamed.borrow_mut().push(Box::new(renamed));
    }

    async fn apply_now(self: Rc<Self>) -> Result<(), String> {
        let window = self.window.upgrade();
        let opener = window.as_ref().and_then(PropertiesWindow::opener);
        if let Some(read_only) = self.read_only.change() {
            self.apply_read_only(read_only, opener.as_ref()).await?;
        }
        if let Some(hide) = self.hidden.change() {
            let Some(opener) = opener else {
                return Err(ox_core::i18n::gettext(
                    "The window that opened these properties is closed.",
                ));
            };
            let parent = window.as_ref().map_or_else(
                || opener.clone().upcast::<gtk::Window>(),
                |window| window.clone().upcast(),
            );
            // Hidden waits while the items are renamed, so it shows what
            // the renames made.
            self.hidden.check.set_sensitive(false);
            let renaming = self.rename_all(&opener, &parent, hide).await;
            self.hidden.check.set_sensitive(true);
            if let Some(window) = &window {
                let names: Vec<String> = self.items.borrow().iter().map(|item| item.name.clone()).collect();
                window.set_title_text(&selection_title(&names));
            }
            if renaming.is_err() {
                // The items renamed before the refusal keep their names: the
                // box compares the choice with what they have now.
                let now = shared(self.items.borrow().iter().map(|item| item.name.starts_with('.')));
                self.hidden.now.set(now);
            }
            renaming?;
            self.hidden.reset(Some(hide));
        }
        Ok(())
    }

    /// Makes every file read-only, or writable again, a folder's files
    /// inside it.
    async fn apply_read_only(&self, read_only: bool, opener: Option<&BrowserWindow>) -> Result<(), String> {
        let items: Vec<(String, bool)> = self
            .items
            .borrow()
            .iter()
            .map(|item| (item.uri.clone(), item.is_folder))
            .collect();
        // A location registered as a previous version since the window
        // opened is never changed (PROP-024).
        for (uri, _) in &items {
            self.versions
                .check_writable(uri)
                .map_err(|refusal| refusal.to_string())?;
        }
        // No drive is formatted while the items change (DEV-012).
        let _writing = match opener {
            Some(opener) => Some(opener.hold_write()?),
            None => None,
        };
        let submitted = self.read_only.wanted.get();
        for (uri, is_folder) in items {
            let request = PermissionRequest {
                mode: ModeChange::ReadOnlyFiles(read_only),
                owner: None,
                group: None,
                recursive: is_folder,
            };
            // Backups inside a folder are left alone too.
            let protected = self.versions.protected_locations();
            apply_outside_in_background(uri, request, Cancellation::new(), protected)
                .await
                .map_err(|error| error.to_string())?;
        }
        let now = (!self.has_folder).then_some(read_only);
        // A box changed while the change was made stays to apply.
        if self.read_only.wanted.get() == submitted {
            self.read_only.reset(now);
        } else {
            self.read_only.now.set(now);
        }
        Ok(())
    }

    /// Adds a leading dot to every name that has none, for `hide`, or
    /// removes it from every name that has one, stopping at the first
    /// refusal; the items renamed before it keep their new names.
    async fn rename_all(
        &self,
        opener: &BrowserWindow,
        parent: &gtk::Window,
        hide: bool,
    ) -> Result<(), String> {
        let count = self.items.borrow().len();
        for index in 0..count {
            let item = self.items.borrow()[index].clone();
            if item.name.starts_with('.') == hide {
                continue;
            }
            let wanted = if hide {
                format!(".{}", item.name)
            } else {
                item.name.trim_start_matches('.').to_owned()
            };
            opener
                .rename_item_at(
                    parent,
                    &item.uri,
                    &item.name,
                    &wanted,
                    item.is_folder || item.links_to_folder,
                )
                .await?;
            let renamed = gio::File::for_uri(&item.uri)
                .parent()
                .map(|folder| folder.child(&wanted).uri().to_string());
            let mut items = self.items.borrow_mut();
            if let Some(renamed) = renamed {
                for follow in self.renamed.borrow().iter() {
                    follow(&item.uri, &renamed);
                }
                items[index].uri = renamed;
            }
            items[index].name = wanted;
        }
        Ok(())
    }
}

impl Pending for SelectionAttributes {
    fn has_changes(&self) -> bool {
        self.read_only.change().is_some() || self.hidden.change().is_some()
    }

    fn apply(self: Rc<Self>) -> ApplyFuture {
        Box::pin(self.apply_now())
    }
}
